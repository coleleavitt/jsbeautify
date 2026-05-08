//! Word extraction from JS AST with named source categorization.
//!
//! Ports `crates/core/wordlist/src/extractors/js_ast_extractor.rs` from jxscout.
//!
//! Extracts words from 4 distinct source categories:
//!   - `js_variable_name`  — let/const/var declarations, parameters
//!   - `js_property_name`  — object keys, member expressions
//!   - `js_function_name`  — function declarations/expressions, methods
//!   - `js_class_name`     — class declarations/expressions
//!
//! Each source can be independently toggled via [`ExtractConfig`].

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use rustc_hash::FxHashSet;

use crate::wordlist::filter;

/// Which JS extraction sources to enable.
#[derive(Debug, Clone)]
pub struct ExtractConfig {
    pub variable_names: bool,
    pub property_names: bool,
    pub function_names: bool,
    pub class_names: bool,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            variable_names: true,
            property_names: true,
            function_names: true,
            class_names: true,
        }
    }
}

/// A word with its source category.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourcedWord {
    pub word: String,
    pub source: &'static str,
}

pub const SOURCE_VARIABLE: &str = "js_variable_name";
pub const SOURCE_PROPERTY: &str = "js_property_name";
pub const SOURCE_FUNCTION: &str = "js_function_name";
pub const SOURCE_CLASS: &str = "js_class_name";

/// Extract a flat de-duplicated word list (legacy API, all sources).
#[must_use]
pub fn extract(source: &str) -> Vec<String> {
    extract_sourced(source, &ExtractConfig::default())
        .into_iter()
        .map(|sw| sw.word)
        .collect()
}

/// Extract words with source categorization.
#[must_use]
pub fn extract_sourced(source: &str, config: &ExtractConfig) -> Vec<SourcedWord> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = WordVisitor {
        config,
        words: FxHashSet::default(),
    };
    visitor.visit_program(&parse.program);

    let mut out: Vec<SourcedWord> = visitor.words.into_iter().collect();
    out.sort_unstable_by(|a, b| a.word.cmp(&b.word));
    out
}

struct WordVisitor<'c> {
    config: &'c ExtractConfig,
    words: FxHashSet<SourcedWord>,
}

impl<'c> WordVisitor<'c> {
    fn push(&mut self, raw: &str, source: &'static str) {
        for token in split_tokens(raw) {
            let norm = filter::normalize(&token);
            if !filter::is_noise(&norm) {
                self.words.insert(SourcedWord { word: norm, source });
            }
        }
    }
}

impl<'a, 'c> Visit<'a> for WordVisitor<'c> {
    // Variable declarations: let x, const y, var z, params
    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        if self.config.variable_names {
            self.push(&it.name, SOURCE_VARIABLE);
        }
        walk::walk_binding_identifier(self, it);
    }

    // Function declarations and named function expressions
    fn visit_function(&mut self, it: &Function<'a>, _flags: oxc_syntax::scope::ScopeFlags) {
        if self.config.function_names {
            if let Some(id) = &it.id {
                self.push(&id.name, SOURCE_FUNCTION);
            }
        }
        walk::walk_function(self, it, _flags);
    }

    // Class declarations and named class expressions
    fn visit_class(&mut self, it: &Class<'a>) {
        if self.config.class_names {
            if let Some(id) = &it.id {
                self.push(&id.name, SOURCE_CLASS);
            }
        }
        // Also extract method names as function names
        if self.config.function_names {
            for elem in &it.body.body {
                if let ClassElement::MethodDefinition(md) = elem {
                    if let PropertyKey::StaticIdentifier(id) = &md.key {
                        self.push(&id.name, SOURCE_FUNCTION);
                    }
                }
            }
        }
        // Property definitions as property names
        if self.config.property_names {
            for elem in &it.body.body {
                if let ClassElement::PropertyDefinition(pd) = elem {
                    if let PropertyKey::StaticIdentifier(id) = &pd.key {
                        self.push(&id.name, SOURCE_PROPERTY);
                    }
                }
            }
        }
        walk::walk_class(self, it);
    }

    // Object property keys
    fn visit_object_expression(&mut self, it: &ObjectExpression<'a>) {
        if self.config.property_names {
            for prop in &it.properties {
                if let ObjectPropertyKind::ObjectProperty(op) = prop {
                    match &op.key {
                        PropertyKey::StaticIdentifier(id) => {
                            self.push(&id.name, SOURCE_PROPERTY);
                        }
                        PropertyKey::StringLiteral(s) => {
                            self.push(&s.value, SOURCE_PROPERTY);
                        }
                        _ => {}
                    }
                }
            }
        }
        walk::walk_object_expression(self, it);
    }

    // Member expression property access (obj.prop)
    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if self.config.property_names {
            self.push(&it.property.name, SOURCE_PROPERTY);
        }
        walk::walk_static_member_expression(self, it);
    }

    // String literals → variable source (general content words)
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        if self.config.variable_names {
            self.push(&it.value, SOURCE_VARIABLE);
        }
        walk::walk_string_literal(self, it);
    }
}

/// Split `s` on `/`, `-`, `_`, `.` and camelCase boundaries.
pub(crate) fn split_tokens(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for segment in s.split(|c| matches!(c, '/' | '-' | '_' | '.' | ' ' | ':' | '?' | '=' | '&' | '#')) {
        if segment.is_empty() {
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_sources() {
        let src = r#"
            class UserService {
                apiKey = "test";
                fetchUser(id) { return this.users[id]; }
            }
            const baseUrl = "/api/v2";
            function loadConfig() {}
        "#;
        let words = extract_sourced(src, &ExtractConfig::default());
        let classes: Vec<_> = words
            .iter()
            .filter(|w| w.source == SOURCE_CLASS)
            .map(|w| &w.word)
            .collect();
        let funcs: Vec<_> = words
            .iter()
            .filter(|w| w.source == SOURCE_FUNCTION)
            .map(|w| &w.word)
            .collect();
        let props: Vec<_> = words
            .iter()
            .filter(|w| w.source == SOURCE_PROPERTY)
            .map(|w| &w.word)
            .collect();

        assert!(classes.contains(&&"user".to_string()) || classes.contains(&&"service".to_string()));
        assert!(
            funcs.contains(&&"load".to_string())
                || funcs.contains(&&"config".to_string())
                || funcs.iter().any(|w| w.contains("fetch"))
        );
        assert!(props.iter().any(|w| w.contains("api") || w.contains("key")));
    }

    #[test]
    fn flat_extract_still_works() {
        let src = "const userName = 'admin';";
        let words = extract(src);
        assert!(words.iter().any(|w| w == "admin" || w == "user" || w == "name"));
    }
}
