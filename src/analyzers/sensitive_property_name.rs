//! Sensitive object-property name detection.
//!
//! Ports `crates/core/analyzers/src/js_ast_sensitive_property_name_analyzer.rs`
//! from jxscout-pro-v2.
//!
//! Walks the oxc AST and tests every *property key* that appears in:
//!   - Object literal expressions  (`{ secret: "..." }`)
//!   - Object assignment patterns  (`const { password } = ...`)
//!   - Class member declarations   (`class Foo { #privateKey = null }`)
//!   - JSX attribute names         (`<Comp authorization={...} />`)
//!
//! The key name is tested against [`super::regex_bank::property_name_rules()`].
//! Only exact-identifier names are tested — computed keys (`obj[expr]`) are
//! skipped because the expression isn't statically known.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType};
use super::regex_bank;

/// Scan `source` for object/class/JSX property names that match jxscout's
/// sensitive-property-name ruleset.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    let mut visitor = PropNameVisitor {
        source,
        file_path,
        file_type,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

struct PropNameVisitor<'a> {
    source: &'a str,
    file_path: Option<&'a str>,
    file_type: FileType,
    out: Vec<AnalyzerMatch>,
}

impl<'a> PropNameVisitor<'a> {
    fn check_key(&mut self, name: &str, start: u32, end: u32) {
        for rule in regex_bank::property_name_rules() {
            if rule.regex.is_match(name) {
                self.out.push(AnalyzerMatch {
                    match_kind: rule.kind.to_string(),
                    match_value: name.to_string(),
                    position: span_to_pos(self.source, start, end),
                    file_type: self.file_type,
                    file_path: self.file_path.map(str::to_string),
                    metadata: None,
                });
            }
        }
    }
}

impl<'a> Visit<'a> for PropNameVisitor<'a> {
    fn visit_object_expression(&mut self, it: &ObjectExpression<'a>) {
        for prop in &it.properties {
            if let ObjectPropertyKind::ObjectProperty(op) = prop {
                match &op.key {
                    PropertyKey::StaticIdentifier(id) => {
                        self.check_key(&id.name, id.span.start, id.span.end);
                    }
                    PropertyKey::StringLiteral(s) => {
                        self.check_key(&s.value, s.span.start, s.span.end);
                    }
                    _ => {}
                }
            }
        }
        walk::walk_object_expression(self, it);
    }

    fn visit_object_pattern(&mut self, it: &ObjectPattern<'a>) {
        for prop in &it.properties {
            match &prop.key {
                PropertyKey::StaticIdentifier(id) => {
                    self.check_key(&id.name, id.span.start, id.span.end);
                }
                PropertyKey::StringLiteral(s) => {
                    self.check_key(&s.value, s.span.start, s.span.end);
                }
                _ => {}
            }
        }
        walk::walk_object_pattern(self, it);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        for elem in &it.body.body {
            match elem {
                ClassElement::PropertyDefinition(pd) => match &pd.key {
                    PropertyKey::StaticIdentifier(id) => {
                        self.check_key(&id.name, id.span.start, id.span.end);
                    }
                    PropertyKey::StringLiteral(s) => {
                        self.check_key(&s.value, s.span.start, s.span.end);
                    }
                    PropertyKey::PrivateIdentifier(pi) => {
                        self.check_key(&pi.name, pi.span.start, pi.span.end);
                    }
                    _ => {}
                },
                ClassElement::MethodDefinition(md) => match &md.key {
                    PropertyKey::StaticIdentifier(id) => {
                        self.check_key(&id.name, id.span.start, id.span.end);
                    }
                    PropertyKey::PrivateIdentifier(pi) => {
                        self.check_key(&pi.name, pi.span.start, pi.span.end);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        walk::walk_class(self, it);
    }

    fn visit_jsx_opening_element(&mut self, it: &JSXOpeningElement<'a>) {
        for attr in &it.attributes {
            if let JSXAttributeItem::Attribute(a) = attr {
                if let JSXAttributeName::Identifier(id) = &a.name {
                    self.check_key(&id.name, id.span.start, id.span.end);
                }
            }
        }
        walk::walk_jsx_opening_element(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secret_key() {
        let src = r#"const obj = { secret: "abc", normal: 1 };"#;
        let matches = analyze(src, None, FileType::Js);
        let kinds: Vec<_> = matches.iter().map(|m| m.match_kind.as_str()).collect();
        assert!(
            kinds.iter().any(|k| k.contains("secret")),
            "expected secret match, got {:?}",
            kinds
        );
    }

    #[test]
    fn detects_authorization_in_destructuring() {
        let src = r#"const { authorization, name } = headers;"#;
        let matches = analyze(src, None, FileType::Js);
        assert!(matches.iter().any(|m| m.match_kind.contains("authorization")));
    }

    #[test]
    fn ignores_benign_keys() {
        let src = r#"const obj = { foo: 1, bar: 2, baz: "x" };"#;
        let matches = analyze(src, None, FileType::Js);
        assert!(matches.is_empty(), "should not flag benign keys: {:?}", matches);
    }
}
