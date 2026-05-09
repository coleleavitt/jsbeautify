//! Content diffing between two JS/HTML file versions.
//!
//! Ports the monitoring change-detection concept from jxscout — compare
//! two versions of a file and report what security-relevant content changed.
//!
//! Unlike jxscout (which only does hash comparison), we provide semantic diff:
//! new endpoints, new secrets, removed secrets, new packages, etc.

use std::collections::HashSet;

use crate::analyzers::match_kind::FileType;
use crate::analyzers::{self, AnalyzerConfig, AnalyzerMatch};

/// Result of comparing two file versions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiffResult {
    /// Findings present in `new` but not in `old`.
    pub added: Vec<AnalyzerMatch>,
    /// Findings present in `old` but not in `new`.
    pub removed: Vec<AnalyzerMatch>,
    /// Findings present in both (unchanged).
    pub unchanged_count: usize,
    /// Whether the content hash changed at all.
    pub content_changed: bool,
    /// SHA-256 of old content.
    pub old_hash: String,
    /// SHA-256 of new content.
    pub new_hash: String,
}

/// Compare two versions of a file and report what changed.
///
/// Runs the full analyzer pipeline on both versions and diffs the results
/// by `(match_kind, match_value)` pairs.
#[must_use]
pub fn diff_versions(old_source: &str, new_source: &str, file_path: Option<&str>, file_type: FileType) -> DiffResult {
    let old_hash = content_fingerprint(old_source);
    let new_hash = content_fingerprint(new_source);

    if old_hash == new_hash {
        return DiffResult {
            added: Vec::new(),
            removed: Vec::new(),
            unchanged_count: 0,
            content_changed: false,
            old_hash,
            new_hash,
        };
    }

    let config = AnalyzerConfig::default();
    let old_matches = analyzers::run_all(old_source, file_path, file_type, &config);
    let new_matches = analyzers::run_all(new_source, file_path, file_type, &config);

    // Key by (kind, value) for set comparison
    let old_keys: HashSet<(String, String)> = old_matches
        .iter()
        .map(|m| (m.match_kind.clone(), m.match_value.clone()))
        .collect();
    let new_keys: HashSet<(String, String)> = new_matches
        .iter()
        .map(|m| (m.match_kind.clone(), m.match_value.clone()))
        .collect();

    let added: Vec<AnalyzerMatch> = new_matches
        .into_iter()
        .filter(|m| !old_keys.contains(&(m.match_kind.clone(), m.match_value.clone())))
        .collect();

    let removed: Vec<AnalyzerMatch> = old_matches
        .into_iter()
        .filter(|m| !new_keys.contains(&(m.match_kind.clone(), m.match_value.clone())))
        .collect();

    let unchanged_count = old_keys.intersection(&new_keys).count();

    DiffResult {
        added,
        removed,
        unchanged_count,
        content_changed: true,
        old_hash,
        new_hash,
    }
}

/// Fast non-cryptographic content fingerprint for change detection.
/// Uses SipHash (DefaultHasher) — NOT a cryptographic hash. Suitable only
/// for detecting whether content changed between two versions within the
/// same process. Not stable across Rust versions or platforms.
fn content_fingerprint(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_no_diff() {
        let src = r#"const x = fetch("/api/users");"#;
        let result = diff_versions(src, src, None, FileType::Js);
        assert!(!result.content_changed);
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
    }

    #[test]
    fn detects_new_endpoint() {
        let old = r#"fetch("/api/users");"#;
        let new = r#"fetch("/api/users"); fetch("/api/admin/secrets");"#;
        let result = diff_versions(old, new, None, FileType::Js);
        assert!(result.content_changed);
        assert!(result.added.iter().any(|m| m.match_value.contains("admin")));
    }

    #[test]
    fn detects_removed_secret() {
        let old = r#"const k = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";"#;
        let new = r#"const k = "redacted";"#;
        let result = diff_versions(old, new, None, FileType::Js);
        assert!(result.content_changed);
        assert!(result.removed.iter().any(|m| m.match_kind == "github_pat"));
    }
}
