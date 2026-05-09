//! Path and hostname extraction from inline source comments.
//!
//! Ports `crates/core/analyzers/src/js_ast_comment_path_analyzer.rs` and
//! `js_ast_comment_hostname_analyzer.rs` from jxscout-pro-v2.
//!
//! jxscout surfaces comment-sourced findings with dedicated `match_kind` values
//! (`comment_path` / `comment_hostname`) separate from AST-literal matches so
//! the analyst knows the provenance.

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::helpers;
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;

const KIND_COMMENT_PATH: &str = "comment_path";
const KIND_COMMENT_HOSTNAME: &str = "comment_hostname";

/// Extract paths and hostnames from `//` and `/* */` comments in `source`.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs()).parse();

    let mut out = Vec::new();

    for comment in parse.program.comments.iter() {
        let raw = comment.span.source_text(source);
        // Strip delimiters.
        let text = raw
            .trim_start_matches("//")
            .trim_start_matches("/*")
            .trim_end_matches("*/")
            .trim();

        let pos = PositionRange::from_byte_range(source, comment.span.start as usize, comment.span.end as usize);

        // Check for URL/path patterns in the comment text.
        for cap in helpers::URL_PATTERN.captures_iter(text) {
            let val = cap.get(1).map(|m| m.as_str()).unwrap_or_else(|| &cap[0]);
            if !helpers::is_path_noise(val) {
                out.push(AnalyzerMatch {
                    match_kind: KIND_COMMENT_PATH.to_string(),
                    match_value: val.to_string(),
                    position: pos.clone(),
                    file_type,
                    file_path: file_path.map(str::to_string),
                    severity: Severity::Info,
                    provenance: Provenance::Comment,
                    metadata: None,
                });
            }
        }

        // Check for hostnames.
        for cap in helpers::HOSTNAME_PATTERN.captures_iter(text) {
            let val = cap.get(1).map(|m| m.as_str()).unwrap_or_else(|| &cap[0]);
            if !helpers::is_hostname_noise(val) && !helpers::is_path_noise(val) {
                out.push(AnalyzerMatch {
                    match_kind: KIND_COMMENT_HOSTNAME.to_string(),
                    match_value: val.to_string(),
                    position: pos.clone(),
                    file_type,
                    file_path: file_path.map(str::to_string),
                    severity: Severity::Info,
                    provenance: Provenance::Comment,
                    metadata: None,
                });
            }
        }
    }

    out
}
