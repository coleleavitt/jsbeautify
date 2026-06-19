//! Dangerous-sink detection: HTML manipulation, location assignment,
//! postMessage, onmessage, dangerouslySetInnerHTML, `URLSearchParams`.
//!
//! Ports the following jxscout analyzers in one consolidated module:
//! - `js_ast_html_manipulation_analyzer.rs`         → `html_manipulation`
//! - `js_ast_location_analyzer.rs`                  → `location_assignment`
//! - `js_ast_onmessage_analyzer.rs`                 → `window_onmessage`
//! - `js_ast_postmessage_analyzer.rs`               → `post_message`
//! - `js_ast_react_dangerously_set_inner_analyzer.rs` → `dangerously_set_inner_html`
//! - `js_ast_url_search_params_analyzer.rs`         → `url_search_params`
//!
//! Output schema is stable: `match_kind` aligns 1-to-1 with jxscout's strings.

pub mod kinds;
#[cfg(test)]
mod tests;
mod visitor;

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::match_kind::{AnalyzerMatch, FileType};
use visitor::SinkVisitor;

/// Run all sink analyzers on `source` in a single AST walk.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = SinkVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}
