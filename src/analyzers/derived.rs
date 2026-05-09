//! Derived analyzer — post-filter existing matches into new match kinds.
//!
//! Ports `DerivedMatchAnalyzerConfiguration` from jxscout-pro-v2.
//!
//! A derived analyzer takes the output of any other analyzer (by `original_match_kind`),
//! applies include/exclude regex filters on the match value, and emits results
//! under a `new_match_kind`.

use super::match_kind::AnalyzerMatch;
use super::regex_bank::severity_for_kind;
use regex::Regex;

/// Configuration for a single derived analyzer.
#[derive(Debug, Clone)]
pub struct DerivedAnalyzer {
    /// The match kind to filter from (e.g. "path", "hostname", "npm_package").
    pub original_match_kind: String,
    /// The new match kind to emit (e.g. "admin_path", "internal_hostname").
    pub new_match_kind: String,
    /// Include patterns — at least one must match the value. Empty = include all.
    pub include_patterns: Vec<Regex>,
    /// Exclude patterns — if any match, the value is dropped.
    pub exclude_patterns: Vec<Regex>,
}

impl DerivedAnalyzer {
    /// Create a new derived analyzer.
    pub fn new(
        original_match_kind: impl Into<String>,
        new_match_kind: impl Into<String>,
        include: &[&str],
        exclude: &[&str],
    ) -> Result<Self, regex::Error> {
        let include_patterns = include.iter().map(|p| Regex::new(p)).collect::<Result<Vec<_>, _>>()?;
        let exclude_patterns = exclude.iter().map(|p| Regex::new(p)).collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            original_match_kind: original_match_kind.into(),
            new_match_kind: new_match_kind.into(),
            include_patterns,
            exclude_patterns,
        })
    }

    /// Apply this derived analyzer to a set of matches.
    /// Returns only the matches that pass the filter, with the new match_kind.
    pub fn apply(&self, matches: &[AnalyzerMatch]) -> Vec<AnalyzerMatch> {
        matches
            .iter()
            .filter(|m| m.match_kind == self.original_match_kind)
            .filter(|m| {
                // Include filter: if patterns exist, at least one must match
                if !self.include_patterns.is_empty() {
                    if !self.include_patterns.iter().any(|re| re.is_match(&m.match_value)) {
                        return false;
                    }
                }
                // Exclude filter: if any matches, drop
                if self.exclude_patterns.iter().any(|re| re.is_match(&m.match_value)) {
                    return false;
                }
                true
            })
            .map(|m| AnalyzerMatch {
                match_kind: self.new_match_kind.clone(),
                match_value: m.match_value.clone(),
                position: m.position.clone(),
                file_type: m.file_type,
                file_path: m.file_path.clone(),
                severity: severity_for_kind(&self.new_match_kind),
                metadata: m.metadata.clone(),
            })
            .collect()
    }
}

/// Apply multiple derived analyzers in sequence to a match set.
pub fn apply_all(derived: &[DerivedAnalyzer], matches: &[AnalyzerMatch]) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    for d in derived {
        out.extend(d.apply(matches));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzers::match_kind::{FileType, Position, PositionRange};

    fn make_match(kind: &str, value: &str) -> AnalyzerMatch {
        AnalyzerMatch {
            match_kind: kind.to_string(),
            match_value: value.to_string(),
            position: PositionRange {
                start: Position { line: 1, column: 0 },
                end: Position { line: 1, column: 10 },
            },
            file_type: FileType::Js,
            file_path: None,
            severity: crate::analyzers::regex_bank::Severity::Info,
            metadata: None,
        }
    }

    #[test]
    fn derived_filters_paths() {
        let derived =
            DerivedAnalyzer::new("path", "admin_path", &["(?i)admin|internal"], &["\\.(css|png|jpg)$"]).unwrap();

        let matches = vec![
            make_match("path", "/api/admin/users"),
            make_match("path", "/api/public/health"),
            make_match("path", "/internal/config"),
            make_match("path", "/admin/style.css"),
            make_match("hostname", "admin.example.com"), // wrong kind, ignored
        ];

        let results = derived.apply(&matches);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|m| m.match_kind == "admin_path"));
        assert!(results.iter().any(|m| m.match_value == "/api/admin/users"));
        assert!(results.iter().any(|m| m.match_value == "/internal/config"));
    }

    #[test]
    fn empty_include_passes_all() {
        let derived = DerivedAnalyzer::new("path", "filtered", &[], &["^/$"]).unwrap();
        let matches = vec![make_match("path", "/api/v2"), make_match("path", "/")];
        let results = derived.apply(&matches);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_value, "/api/v2");
    }
}
