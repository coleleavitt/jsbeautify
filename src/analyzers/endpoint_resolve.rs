//! Dynamic endpoint resolution: recover concrete paths from built-up URLs.
//!
//! Many endpoints are not string literals — they're assembled from a base plus
//! interpolation or concatenation (`` `/api/${ver}/users` ``, `"/api/" + ver +
//! "/users"`). A naive path extractor either misses these or reports a fragment.
//! This pass folds the static parts of such expressions into a single
//! **resolved endpoint** with `${…}` placeholders preserved where a value can't
//! be determined statically, plus substitutes simple in-scope constants
//! (`const VER = "v2"`) when their value is a literal.
//!
//! Emits `resolved_endpoint` findings — the concrete-as-possible path — so the
//! IDOR / api analyzers and the recon engine see the real route, not a fragment.

#![allow(clippy::doc_markdown)]

use std::collections::HashMap;

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, VariableDeclarator};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

use super::ast_walker::span_to_pos;
use super::match_kind::{AnalyzerMatch, FileType, Provenance};
use super::regex_bank::Severity;

/// Resolve dynamic endpoint expressions in `source` into concrete paths.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let allocator = Allocator::default();
    let parse = Parser::new(&allocator, source, SourceType::mjs().with_jsx(true)).parse();

    // Pass 1: collect simple `const/let NAME = "literal"` bindings.
    let mut consts = ConstCollector { consts: HashMap::new() };
    consts.visit_program(&parse.program);

    // Pass 2: fold template literals / string concatenations into endpoints.
    let mut visitor = EndpointVisitor {
        source,
        file_path,
        file_type,
        consts: &consts.consts,
        out: Vec::new(),
    };
    visitor.visit_program(&parse.program);
    visitor.out
}

/// Collects `NAME -> "literal"` for simple const string bindings.
struct ConstCollector {
    consts: HashMap<String, String>,
}

impl<'a> Visit<'a> for ConstCollector {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (Some(Expression::StringLiteral(s)), oxc_ast::ast::BindingPattern::BindingIdentifier(id)) =
            (&decl.init, &decl.id)
        {
            self.consts.insert(id.name.to_string(), s.value.to_string());
        }
        walk::walk_variable_declarator(self, decl);
    }
}

struct EndpointVisitor<'s, 'c> {
    source: &'s str,
    file_path: Option<&'s str>,
    file_type: FileType,
    consts: &'c HashMap<String, String>,
    out: Vec<AnalyzerMatch>,
}

impl EndpointVisitor<'_, '_> {
    fn maybe_emit(&mut self, resolved: &str, span: Span) {
        // Only surface things that look like an endpoint path.
        let looks_path = resolved.starts_with('/') || resolved.contains("/api") || resolved.contains("://");
        let has_segment = resolved.trim_matches('/').contains('/') || resolved.len() > 4;
        if looks_path && has_segment {
            self.out.push(
                AnalyzerMatch::new(
                    "resolved_endpoint",
                    resolved.to_string(),
                    span_to_pos(self.source, span.start, span.end),
                    self.file_type,
                    self.file_path.map(String::from),
                )
                .with_provenance(Provenance::Derived)
                .with_severity(Severity::Info),
            );
        }
    }
}

impl<'a> Visit<'a> for EndpointVisitor<'_, '_> {
    fn visit_expression(&mut self, expr: &Expression<'a>) {
        // Only template literals + `+` concatenations build dynamic endpoints;
        // every other expression is walked through without folding.
        if matches!(expr, Expression::TemplateLiteral(_) | Expression::BinaryExpression(_))
            && let Some(resolved) = fold_expression(expr, self.consts)
        {
            self.maybe_emit(&resolved, expr.span());
        }
        walk::walk_expression(self, expr);
    }
}

/// Fold an expression into a resolved string, or `None` if nothing static.
/// Unknown dynamic parts become `${…}` placeholders.
fn fold_expression(expr: &Expression<'_>, consts: &HashMap<String, String>) -> Option<String> {
    match expr {
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        Expression::Identifier(id) => Some(
            consts
                .get(id.name.as_str())
                .cloned()
                .unwrap_or_else(|| format!("${{{}}}", id.name)),
        ),
        Expression::TemplateLiteral(t) => {
            let mut out = String::new();
            // Interleave quasis (static parts) with expressions.
            for (i, quasi) in t.quasis.iter().enumerate() {
                out.push_str(quasi.value.raw.as_str());
                if let Some(e) = t.expressions.get(i) {
                    out.push_str(&fold_expression(e, consts).unwrap_or_else(|| "${…}".to_string()));
                }
            }
            Some(out)
        }
        Expression::BinaryExpression(b) if matches!(b.operator, oxc_syntax::operator::BinaryOperator::Addition) => {
            let l = fold_expression(&b.left, consts)?;
            let r = fold_expression(&b.right, consts)?;
            Some(format!("{l}{r}"))
        }
        Expression::ParenthesizedExpression(p) => fold_expression(&p.expression, consts),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(src: &str) -> Vec<String> {
        analyze(src, None, FileType::Js)
            .into_iter()
            .filter(|m| m.match_kind == "resolved_endpoint")
            .map(|m| m.match_value)
            .collect()
    }

    #[test]
    fn folds_template_with_const() {
        let src = r#"const ver = "v2"; const u = `/api/${ver}/users`;"#;
        let r = resolved(src);
        assert!(r.iter().any(|s| s == "/api/v2/users"), "{r:?}");
    }

    #[test]
    fn keeps_placeholder_for_unknown() {
        let src = "function f(id){ return `/api/users/${id}`; }";
        let r = resolved(src);
        assert!(r.iter().any(|s| s == "/api/users/${id}"), "{r:?}");
    }

    #[test]
    fn folds_string_concatenation() {
        let src = r#"const base = "/api"; var u = base + "/orders/" + "list";"#;
        let r = resolved(src);
        assert!(r.iter().any(|s| s == "/api/orders/list"), "{r:?}");
    }

    #[test]
    fn ignores_non_endpoint_strings() {
        let src = r#"var msg = `hello ${name}`;"#;
        assert!(resolved(src).is_empty());
    }
}
