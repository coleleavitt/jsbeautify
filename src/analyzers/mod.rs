//! Security analysis module.
//!
//! Ports the 14 AST analyzers from `crates/core/analyzers/` in jxscout-pro-v2.
//! All analyzers operate on a raw JS/HTML/SourceMap string and return a flat
//! `Vec<AnalyzerMatch>`.  Output schema is drop-in compatible with jxscout's
//! `get-matches --json` format.
//!
//! ## Quick start
//!
//! ```rust
//! use js_beautify_rs::analyzers::{run_all, AnalyzerConfig, match_kind::FileType};
//!
//! let src = r#"const SECRET_KEY = "ghp_abc123def456ghi789jkl012mno345pqr6"; fetch("/api/users");"#;
//! let matches = run_all(src, None, FileType::Js, &AnalyzerConfig::default());
//! println!("{} findings", matches.len());
//! ```

pub mod api_extractor;
pub mod ast_walker;
pub mod comment_extractor;
pub mod derived;
pub mod graphql_extractor;
pub mod helpers;
pub mod hostname_extractor;
pub mod html_script_extractor;
pub mod match_kind;
pub mod npm_package_extractor;
pub mod package_json_analyzer;
pub mod path_extractor;
pub mod regex_bank;
pub mod secret_scanner;
pub mod sensitive_property_name;
pub mod sinks;

// Framework / build-tool recon: per-framework signature analyzers + dispatcher.
pub mod framework_recon;
pub mod fw_module_federation;
pub mod fw_next_rsc;
pub mod fw_preact;
pub mod fw_react;
pub mod fw_redux;
pub mod fw_rollup;
pub mod fw_sentry;
pub mod fw_solid;
pub mod fw_svelte;
pub mod fw_vite;
pub mod fw_vue;
pub mod fw_webpack;
pub mod fw_zustand;

pub use match_kind::{AnalyzerMatch, FileType, PositionRange};

/// Deduplicate `matches` so each unique `(match_kind, match_value, file_path,
/// metadata)` tuple appears only once. Mirrors jxscout's
/// `finding_per_unique_value` setting (REPORT_03 §"Match → Finding promotion").
///
/// The first occurrence wins — its position/severity is preserved. Order is
/// otherwise stable.
#[must_use]
pub fn dedup_unique_values(matches: Vec<AnalyzerMatch>) -> Vec<AnalyzerMatch> {
    let mut seen: rustc_hash::FxHashSet<u64> = rustc_hash::FxHashSet::default();
    let mut out = Vec::with_capacity(matches.len());
    for m in matches {
        if seen.insert(m.dedup_hash()) {
            out.push(m);
        }
    }
    out
}

/// Controls which analyzers are enabled in [`run_all`].
///
/// Default: every analyzer enabled.
#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    pub secrets: bool,
    pub sensitive_props: bool,
    pub paths_hostnames: bool,
    pub graphql: bool,
    pub sinks: bool,
    pub npm_packages: bool,
    pub comments: bool,
    pub framework_recon: bool,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            secrets: true,
            sensitive_props: true,
            paths_hostnames: true,
            graphql: true,
            sinks: true,
            npm_packages: true,
            comments: true,
            framework_recon: true,
        }
    }
}

impl AnalyzerConfig {
    /// Only enable the secret/credential scanner.
    pub fn secrets_only() -> Self {
        Self {
            secrets: true,
            sensitive_props: false,
            paths_hostnames: false,
            graphql: false,
            sinks: false,
            npm_packages: false,
            comments: false,
            framework_recon: false,
        }
    }

    /// Only enable the sink/XSS detectors.
    pub fn sinks_only() -> Self {
        Self {
            secrets: false,
            sensitive_props: false,
            paths_hostnames: false,
            graphql: false,
            sinks: true,
            npm_packages: false,
            comments: false,
            framework_recon: false,
        }
    }
}

/// Run every enabled analyzer over `source` and return a unified match list.
///
/// Matches from different analyzers are concatenated in a deterministic order:
/// secrets → sensitive props → paths/hostnames → GraphQL → sinks → npm → comments.
#[must_use]
pub fn run_all(
    source: &str,
    file_path: Option<&str>,
    file_type: FileType,
    config: &AnalyzerConfig,
) -> Vec<AnalyzerMatch> {
    let mut out: Vec<AnalyzerMatch> = Vec::new();

    if config.secrets {
        out.extend(secret_scanner::scan(source, file_path, file_type));
    }
    if config.sensitive_props {
        out.extend(sensitive_property_name::analyze(source, file_path, file_type));
    }
    if config.paths_hostnames {
        out.extend(path_extractor::analyze(source, file_path, file_type));
    }
    if config.graphql {
        out.extend(graphql_extractor::analyze(source, file_path, file_type));
    }
    if config.sinks {
        out.extend(sinks::analyze(source, file_path, file_type));
    }
    if config.npm_packages {
        out.extend(npm_package_extractor::analyze(source, file_path, file_type));
        out.extend(package_json_analyzer::analyze(source, file_path, file_type));
    }
    if config.comments {
        out.extend(comment_extractor::analyze(source, file_path, file_type));
    }

    if config.framework_recon {
        out.extend(framework_recon::analyze(source, file_path, file_type));
    }

    // Always run API call + storage extraction
    out.extend(api_extractor::analyze(source, file_path, file_type));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
        import React from 'react';
        import axios from 'axios';

        const config = {
            apiKey: "AIzaSy_REDACTED_EXAMPLE_KEY",
            secret: "super-secret-value",
            authorization: "Bearer token-here",
        };

        export function UserProfile() {
            return <div dangerouslySetInnerHTML={{ __html: userInput }} />;
        }

        async function loadUser(id) {
            const resp = await fetch(`/api/v2/users/${id}`);
            const { authorization } = resp.headers;
            document.getElementById('output').innerHTML = resp.data;
            return resp;
        }

        // See https://api.example.com/docs for details
        const GQL = `
            query GetUser($id: ID!) {
                user(id: $id) { name email }
            }
        `;
    "#;

    #[test]
    fn run_all_finds_multiple_kinds() {
        let matches = run_all(FIXTURE, Some("test.js"), FileType::Js, &AnalyzerConfig::default());
        let kinds: std::collections::HashSet<&str> = matches.iter().map(|m| m.match_kind.as_str()).collect();

        // Should fire google API key pattern.
        assert!(
            kinds
                .iter()
                .any(|k| k.contains("google") || k.contains("api_key") || k.contains("secret")),
            "expected secret match; got kinds: {:?}",
            kinds
        );
        // Should fire dangerouslySetInnerHTML.
        assert!(
            kinds.contains("dangerously_set_inner_html"),
            "expected sink match; got: {:?}",
            kinds
        );
        // Should fire graphql.
        assert!(kinds.contains("graphql"), "expected graphql match; got: {:?}", kinds);
        // Should fire npm (axios, react).
        assert!(kinds.contains("npm_package"), "expected npm match; got: {:?}", kinds);
    }

    #[test]
    fn empty_source_returns_empty() {
        let matches = run_all("", None, FileType::Js, &AnalyzerConfig::default());
        assert!(matches.is_empty());
    }
}
