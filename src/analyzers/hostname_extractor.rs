//! Hostname-only extraction analyzer.
//!
//! In jxscout, `js_ast_hostname_analyzer.rs` is a separate crate-level
//! analyzer from `js_ast_path_analyzer.rs` so users can toggle it
//! independently and apply hostname-specific noise lists. Because both rely
//! on the same string-literal walk, we keep the heavy lifting in
//! [`super::path_extractor`] and surface a separable view here.
//!
//! Use [`analyze`] to get *only* `hostname` and `comment_hostname` matches
//! — paths and URLs are filtered out. This is what `--extract-hostnames`
//! should call.

use super::match_kind::{AnalyzerMatch, FileType};

/// Run hostname extraction over `source`.
///
/// Internally re-uses [`super::path_extractor::analyze`] and filters its
/// output to the hostname-specific match kinds.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    super::path_extractor::analyze(source, file_path, file_type)
        .into_iter()
        .filter(|m| matches!(m.match_kind.as_str(), "hostname" | "comment_hostname"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_only_hostnames() {
        let src = r#"
            const api = "https://api.example.com/v1/users";
            const cdn = "cdn.example.com";
            const path = "/internal/admin";
            // see https://docs.target.com for spec
        "#;
        let matches = analyze(src, Some("t.js"), FileType::Js);
        for m in &matches {
            assert!(matches!(m.match_kind.as_str(), "hostname" | "comment_hostname"),);
        }
        let values: Vec<&str> = matches.iter().map(|m| m.match_value.as_str()).collect();
        assert!(
            values
                .iter()
                .any(|v| v.contains("example.com") || v.contains("target.com"))
        );
    }

    #[test]
    fn filters_out_paths_and_urls() {
        let src = r#"const x = "/api/v2/users";"#;
        let matches = analyze(src, None, FileType::Js);
        // No bare paths should leak through
        assert!(matches.iter().all(|m| m.match_kind != "path"));
        assert!(matches.iter().all(|m| m.match_kind != "url"));
    }
}
