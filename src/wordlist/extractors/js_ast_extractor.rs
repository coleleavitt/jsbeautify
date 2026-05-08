//! Word extraction from JS string literals and identifiers via oxc AST.
//!
//! Ports `crates/core/wordlist/src/extractors/js_ast_extractor.rs`.
//!
//! Collects unique words from:
//!   - String literal values
//!   - Identifier names
//!   - Import/export source paths (the package-name portion only)
//!
//! Words are split on `/`, `-`, `_`, `.`, camelCase boundaries,
//! then filtered through `super::super::filter`.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use rustc_hash::FxHashSet;

use crate::wordlist::filter;

/// Extract a de-duplicated, noise-filtered word list from `source`.
#[must_use]
pub fn extract(source: &str) -> Vec<String> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs()).parse();

    let mut visitor = WordVisitor {
        words: FxHashSet::default(),
    };
    visitor.visit_program(&parse.program);

    let mut out: Vec<String> = visitor.words.into_iter().collect();
    out.sort_unstable();
    out
}

struct WordVisitor {
    words: FxHashSet<String>,
}

impl WordVisitor {
    fn push(&mut self, raw: &str) {
        for token in split_tokens(raw) {
            let norm = filter::normalize(&token);
            if !filter::is_noise(&norm) {
                self.words.insert(norm);
            }
        }
    }
}

impl<'a> Visit<'a> for WordVisitor {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.push(&it.value);
        walk::walk_string_literal(self, it);
    }

    fn visit_identifier_name(&mut self, it: &IdentifierName<'a>) {
        self.push(&it.name);
        walk::walk_identifier_name(self, it);
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.push(&it.name);
        walk::walk_binding_identifier(self, it);
    }
}

/// Split `s` on `/`, `-`, `_`, `.` and also camelCase boundaries.
fn split_tokens(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    // First split on delimiters.
    for segment in s.split(|c| matches!(c, '/' | '-' | '_' | '.' | ' ' | ':')) {
        if segment.is_empty() {
            continue;
        }
        // Then split camelCase.
        let mut current = String::new();
        for ch in segment.chars() {
            if ch.is_uppercase() && !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            current.push(ch);
        }
        if !current.is_empty() {
            parts.push(current);
        }
    }
    parts
}
