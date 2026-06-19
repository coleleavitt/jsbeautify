//! Cross-environment bundle diff — compares webpack chunks between environments.
//!
//! Schwab runs multiple environments with different bundle versions:
//! - prod/cert/nr: identical bundles (hash 221d1c6a7aa765d2)
//! - qa/pvs/nr-pvs: one version ahead (hash f4b7c63e2450bc60)
//! - int2: Angular 10 legacy (completely different)
//! - dev: skeleton/empty
//!
//! Diffing qa vs prod reveals what's about to ship — new endpoints, features,
//! security control changes. The deterministic module IDs and content hashes
//! make chunk-level diffing precise.

use rustc_hash::FxHashMap;
use std::fmt::Write as _;

/// A webpack chunk with its metadata.
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    pub id: u32,
    pub hash: String,
    pub name: Option<String>,
    pub size_bytes: usize,
}

/// Result of comparing two environments' chunk manifests.
#[derive(Debug)]
pub struct BundleDiffResult {
    /// Chunks present in target but not source (new features)
    pub added_chunks: Vec<ChunkInfo>,
    /// Chunks present in source but not target (removed features)
    pub removed_chunks: Vec<ChunkInfo>,
    /// Chunks with same ID but different hash (modified)
    pub modified_chunks: Vec<(ChunkInfo, ChunkInfo)>,
    /// Chunks identical in both
    pub unchanged_count: usize,
}

/// Parse a webpack chunk manifest from the runtime's `t.u` function.
/// Input format: `{274:"180fc663f5d1ba26",682:"e2d70eae5eb200d5",...}`
#[must_use]
pub fn parse_chunk_manifest(source: &str) -> Vec<ChunkInfo> {
    use regex::Regex;
    use std::sync::LazyLock;

    static CHUNK_HASH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\d+):\s*"([0-9a-f]{16,})""#).unwrap());

    static CHUNK_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\d+):\s*"([a-z][\w-]+)""#).unwrap());

    let mut chunks: FxHashMap<u32, ChunkInfo> = FxHashMap::default();

    // Extract chunk ID → hash mappings
    for caps in CHUNK_HASH_RE.captures_iter(source) {
        let id: u32 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let hash = caps.get(2).map_or("", |m| m.as_str()).to_string();

        chunks.entry(id).or_insert_with(|| ChunkInfo {
            id,
            hash,
            name: None,
            size_bytes: 0,
        });
    }

    // Extract chunk ID → name mappings (e.g. 1755: "polyfills-css-shim")
    for caps in CHUNK_NAME_RE.captures_iter(source) {
        let id: u32 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let name = caps.get(2).map_or("", |m| m.as_str()).to_string();

        if let Some(chunk) = chunks.get_mut(&id) {
            chunk.name = Some(name);
        }
    }

    let mut result: Vec<ChunkInfo> = chunks.into_values().collect();
    result.sort_by_key(|c| c.id);
    result
}

/// Compare two chunk manifests and produce a diff.
#[must_use]
pub fn diff_manifests(source: &[ChunkInfo], target: &[ChunkInfo]) -> BundleDiffResult {
    let source_map: FxHashMap<u32, &ChunkInfo> = source.iter().map(|c| (c.id, c)).collect();
    let target_map: FxHashMap<u32, &ChunkInfo> = target.iter().map(|c| (c.id, c)).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = 0usize;

    // Find added and modified
    for (id, target_chunk) in &target_map {
        if let Some(source_chunk) = source_map.get(id) {
            if source_chunk.hash == target_chunk.hash {
                unchanged += 1;
            } else {
                modified.push(((*source_chunk).clone(), (*target_chunk).clone()));
            }
        } else {
            added.push((*target_chunk).clone());
        }
    }

    // Find removed
    for (id, source_chunk) in &source_map {
        if !target_map.contains_key(id) {
            removed.push((*source_chunk).clone());
        }
    }

    added.sort_by_key(|c| c.id);
    removed.sort_by_key(|c| c.id);
    modified.sort_by_key(|(s, _)| s.id);

    BundleDiffResult {
        added_chunks: added,
        removed_chunks: removed,
        modified_chunks: modified,
        unchanged_count: unchanged,
    }
}

/// Format diff result as readable report.
#[must_use]
pub fn format_bundle_diff(diff: &BundleDiffResult, source_env: &str, target_env: &str) -> String {
    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    let _ = writeln!(output, "// ║  BUNDLE DIFF: {source_env} → {target_env}");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");
    let _ = writeln!(
        output,
        "// ║ Unchanged: {}  Modified: {}  Added: {}  Removed: {}\n",
        diff.unchanged_count,
        diff.modified_chunks.len(),
        diff.added_chunks.len(),
        diff.removed_chunks.len()
    );

    if !diff.added_chunks.is_empty() {
        output.push_str("// ║\n// ║ NEW CHUNKS (about to ship):\n");
        for chunk in &diff.added_chunks {
            let name = chunk.name.as_deref().unwrap_or("");
            let _ = writeln!(output, "// ║   + chunk {} ({}) [{}]", chunk.id, chunk.hash, name);
        }
    }

    if !diff.removed_chunks.is_empty() {
        output.push_str("// ║\n// ║ REMOVED CHUNKS:\n");
        for chunk in &diff.removed_chunks {
            let name = chunk.name.as_deref().unwrap_or("");
            let _ = writeln!(output, "// ║   - chunk {} ({}) [{}]", chunk.id, chunk.hash, name);
        }
    }

    if !diff.modified_chunks.is_empty() {
        output.push_str("// ║\n// ║ MODIFIED CHUNKS (code changed):\n");
        for (src, tgt) in &diff.modified_chunks {
            let name = src.name.as_deref().or(tgt.name.as_deref()).unwrap_or("");
            let _ = writeln!(
                output,
                "// ║   ~ chunk {} {} → {} [{}]\n",
                src.id,
                &src.hash[..8],
                &tgt.hash[..8],
                name
            );
        }
    }

    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest() {
        let source = r#"
            t.u = (e) => ({1755:"polyfills-css-shim"}[e]||e)+"."+{
                274:"180fc663f5d1ba26",
                682:"e2d70eae5eb200d5",
                1755:"5ee1ca9d9826cf31",
                3832:"9907b7ed927a6d7d"
            }[e]+".js";
        "#;

        let chunks = parse_chunk_manifest(source);
        assert!(chunks.len() >= 4);
        assert!(chunks.iter().any(|c| c.id == 274 && c.hash == "180fc663f5d1ba26"));
        assert!(chunks.iter().any(|c| c.id == 3832 && c.hash == "9907b7ed927a6d7d"));
    }

    #[test]
    fn test_diff_manifests() {
        let source = vec![
            ChunkInfo {
                id: 100,
                hash: "aaa".into(),
                name: None,
                size_bytes: 0,
            },
            ChunkInfo {
                id: 200,
                hash: "bbb".into(),
                name: None,
                size_bytes: 0,
            },
            ChunkInfo {
                id: 300,
                hash: "ccc".into(),
                name: None,
                size_bytes: 0,
            },
        ];
        let target = vec![
            ChunkInfo {
                id: 100,
                hash: "aaa".into(),
                name: None,
                size_bytes: 0,
            }, // unchanged
            ChunkInfo {
                id: 200,
                hash: "xxx".into(),
                name: None,
                size_bytes: 0,
            }, // modified
            ChunkInfo {
                id: 400,
                hash: "ddd".into(),
                name: None,
                size_bytes: 0,
            }, // added
        ];

        let diff = diff_manifests(&source, &target);
        assert_eq!(diff.unchanged_count, 1);
        assert_eq!(diff.modified_chunks.len(), 1);
        assert_eq!(diff.added_chunks.len(), 1);
        assert_eq!(diff.removed_chunks.len(), 1);
        assert_eq!(diff.added_chunks[0].id, 400);
        assert_eq!(diff.removed_chunks[0].id, 300);
    }
}
